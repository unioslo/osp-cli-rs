use super::{
    ConfigCommandContext, ConfigReadContext, ConfigScopeTarget, ConfigStore, ConfigStoreTarget,
    ConfigWriteTarget, config_diagnostics_rows, config_get_rows, config_store_name,
    resolve_config_scopes, resolve_config_store, resolve_scope_target, resolve_store_target,
    resolve_terminal_selector, run_config_get, run_config_set, run_config_unset,
    secrets_permissions_diagnostic, session_scoped_value, validate_store_key,
    validate_write_scopes,
};
use crate::app::ReplCommandOutput;
use crate::app::{RuntimeContext, TerminalKind, UiState};
use crate::cli::{ConfigScopeArgs, ConfigSetArgs, ConfigStoreArgs, ConfigUnsetArgs};
use crate::config::{
    ConfigLayer, ConfigResolver, ResolveOptions, ResolvedConfig, RuntimeLoadOptions, Scope,
};
use crate::core::output::OutputFormat;
use crate::core::output_model::OutputItems;
use crate::ui::RenderSettings;
use crate::ui::messages::MessageBuffer;
use crate::ui::messages::MessageLevel;
use crate::ui::theme_catalog::ThemeCatalog;
use std::path::PathBuf;
use std::sync::Mutex;

fn build_resolved_config(defaults: ConfigLayer, terminal: TerminalKind) -> ResolvedConfig {
    let mut defaults = defaults;
    defaults.set("theme.path", Vec::<String>::new());
    if matches!(terminal, TerminalKind::Repl) {
        defaults.set("repl.history.path", "/tmp/osp-cli-config-history.jsonl");
    }
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver
        .resolve(ResolveOptions::default().with_terminal(terminal.as_config_terminal()))
        .expect("test config should resolve")
}

fn test_ui_state(format: OutputFormat) -> UiState {
    UiState::new(RenderSettings::test_plain(format), MessageLevel::Success, 0)
}

struct ConfigTestFixture {
    context: RuntimeContext,
    config: ResolvedConfig,
    ui: UiState,
    themes: ThemeCatalog,
    config_overrides: ConfigLayer,
    product_defaults: ConfigLayer,
    runtime_load: RuntimeLoadOptions,
}

impl ConfigTestFixture {
    fn new(terminal: TerminalKind) -> Self {
        Self::with_format(terminal, OutputFormat::Table)
    }

    fn with_format(terminal: TerminalKind, format: OutputFormat) -> Self {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "ops");
        Self::with_defaults_and_format(terminal, defaults, format)
    }

    fn with_defaults(terminal: TerminalKind, defaults: ConfigLayer) -> Self {
        Self::with_defaults_and_format(terminal, defaults, OutputFormat::Table)
    }

    fn with_defaults_and_format(
        terminal: TerminalKind,
        defaults: ConfigLayer,
        format: OutputFormat,
    ) -> Self {
        Self {
            context: RuntimeContext::new(None, terminal, None),
            config: build_resolved_config(defaults, terminal),
            ui: test_ui_state(format),
            themes: ThemeCatalog::default(),
            config_overrides: ConfigLayer::default(),
            product_defaults: ConfigLayer::default(),
            runtime_load: RuntimeLoadOptions::defaults_only(),
        }
    }

    fn read(&self) -> ConfigReadContext<'_> {
        ConfigReadContext {
            context: &self.context,
            config: &self.config,
            ui: &self.ui,
            themes: &self.themes,
            config_overrides: &self.config_overrides,
            product_defaults: &self.product_defaults,
            runtime_load: self.runtime_load,
        }
    }

    fn command(&mut self) -> ConfigCommandContext<'_> {
        ConfigCommandContext {
            context: &self.context,
            config: &self.config,
            ui: &self.ui,
            themes: &self.themes,
            config_overrides: &mut self.config_overrides,
            product_defaults: &self.product_defaults,
            runtime_load: self.runtime_load,
        }
    }
}

fn write_target(scope: ConfigScopeTarget) -> ConfigWriteTarget {
    ConfigWriteTarget {
        scope,
        terminal: None,
        store: ConfigStoreTarget::Default,
    }
}

fn config_set_args(key: &str, value: &str) -> ConfigSetArgs {
    ConfigSetArgs {
        key: key.to_string(),
        value: value.to_string(),
        scope: ConfigScopeArgs::default(),
        store: ConfigStoreArgs::default(),
        yes: false,
        explain: false,
        dry_run: false,
    }
}

fn config_unset_args(key: &str) -> ConfigUnsetArgs {
    ConfigUnsetArgs {
        key: key.to_string(),
        scope: ConfigScopeArgs::default(),
        store: ConfigStoreArgs::default(),
        dry_run: false,
    }
}

#[test]
fn alias_commands_reuse_resolved_config_and_session_mutation_unit() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "ops");
    defaults.set("alias.hosts", "ldap host ${1:login*}");
    let mut fixture = ConfigTestFixture::with_defaults(TerminalKind::Repl, defaults);

    let listed = super::run_alias_command(
        fixture.command(),
        crate::cli::AliasArgs {
            command: crate::cli::AliasCommands::List(crate::cli::AliasListArgs { sources: true }),
        },
    )
    .expect("alias list should succeed");
    let Some(ReplCommandOutput::Output(listed)) = listed.output else {
        panic!("alias list should return structured output");
    };
    let OutputItems::Rows(rows) = listed.output.items else {
        panic!("alias list should return rows");
    };
    assert_eq!(rows[0]["name"], "hosts");
    assert_eq!(rows[0]["template"], "ldap host ${1:login*}");
    assert_eq!(rows[0]["source"], "defaults");

    let invalid = super::run_alias_command(
        fixture.command(),
        crate::cli::AliasArgs {
            command: crate::cli::AliasCommands::Add(crate::cli::AliasAddArgs {
                name: "broken".to_string(),
                template: "ldap host ${".to_string(),
                scope: ConfigScopeArgs::default(),
                store: ConfigStoreArgs::default(),
                dry_run: false,
            }),
        },
    )
    .expect_err("invalid alias template should be rejected before storage");
    assert!(
        invalid
            .to_string()
            .contains("invalid alias placeholder syntax")
    );

    super::run_alias_command(
        fixture.command(),
        crate::cli::AliasArgs {
            command: crate::cli::AliasCommands::Add(crate::cli::AliasAddArgs {
                name: "lsng".to_string(),
                template: "ldap netgroup ${1} --value | P members".to_string(),
                scope: ConfigScopeArgs::default(),
                store: ConfigStoreArgs {
                    session: true,
                    ..ConfigStoreArgs::default()
                },
                dry_run: false,
            }),
        },
    )
    .expect("valid alias should use config set");
    assert!(
        fixture
            .config_overrides
            .entries()
            .iter()
            .any(|entry| entry.key == "alias.lsng")
    );

    super::run_alias_command(
        fixture.command(),
        crate::cli::AliasArgs {
            command: crate::cli::AliasCommands::Remove(crate::cli::AliasRemoveArgs {
                name: "lsng".to_string(),
                scope: ConfigScopeArgs::default(),
                store: ConfigStoreArgs {
                    session: true,
                    ..ConfigStoreArgs::default()
                },
                dry_run: false,
            }),
        },
    )
    .expect("alias remove should use config unset");
    assert!(fixture.config_overrides.entries().is_empty());
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
        resolve_config_store(ConfigTestFixture::new(TerminalKind::Repl).read(), &args),
        ConfigStore::Session
    ));
    assert!(matches!(
        resolve_config_store(ConfigTestFixture::new(TerminalKind::Cli).read(), &args),
        ConfigStore::Config
    ));

    let fixture = ConfigTestFixture::new(TerminalKind::Repl);
    let repl = fixture.read();
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
    let fixture = ConfigTestFixture::new(TerminalKind::Repl);
    let repl = fixture.read();
    let terminal = repl.context.terminal_kind().as_config_terminal();
    assert_eq!(
        resolve_terminal_selector(terminal, Some(crate::app::CURRENT_TERMINAL_SENTINEL)),
        Some("repl".to_string())
    );
    assert_eq!(resolve_terminal_selector(terminal, Some("  ")), None);
    assert_eq!(
        resolve_terminal_selector(terminal, Some("CLI")),
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
    let cli_fixture = ConfigTestFixture::new(TerminalKind::Cli);
    let cli = cli_fixture.read();

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
        ConfigTestFixture::new(TerminalKind::Cli).read(),
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
    let fixture = ConfigTestFixture::with_defaults(TerminalKind::Cli, defaults);
    let context = fixture.read();

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
        ConfigTestFixture::new(TerminalKind::Cli).read(),
        &crate::cli::ConfigGetArgs {
            key: "profile.default".to_string(),
            output: crate::cli::ConfigReadOutputArgs {
                sources: true,
                raw: false,
            },
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
    let fixture = ConfigTestFixture::with_defaults(TerminalKind::Cli, defaults);
    let context = fixture.read();

    let alias_result = run_config_get(
        context,
        crate::cli::ConfigGetArgs {
            key: "lookup".to_string(),
            output: crate::cli::ConfigReadOutputArgs {
                sources: false,
                raw: false,
            },
        },
    )
    .expect("alias lookup should succeed");
    assert_eq!(alias_result.exit_code, 0);
    assert!(matches!(
        alias_result.output,
        Some(ReplCommandOutput::Output(_))
    ));

    let missing_result = run_config_get(
        context,
        crate::cli::ConfigGetArgs {
            key: "missing.key".to_string(),
            output: crate::cli::ConfigReadOutputArgs {
                sources: false,
                raw: false,
            },
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
    let mut set_fixture = ConfigTestFixture::new(TerminalKind::Repl);
    let set_result = run_config_set(set_fixture.command(), config_set_args("ui.format", "json"))
        .expect("session config set should succeed");

    assert_eq!(set_result.exit_code, 0);
    assert!(matches!(
        set_result.output,
        Some(ReplCommandOutput::Output(_))
    ));
    assert!(
        set_result
            .messages
            .render_grouped(MessageLevel::Success)
            .contains("set value for ui.format")
    );

    let mut unset_fixture = ConfigTestFixture::new(TerminalKind::Repl);
    let unset_context = unset_fixture.command();
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
        Some(ReplCommandOutput::Output(_))
    ));
    assert!(
        unset_result
            .messages
            .render_grouped(MessageLevel::Success)
            .contains("unset value for ui.format")
    );

    let mut explain_args = config_set_args("ui.format", "json");
    explain_args.explain = true;
    let mut explain_fixture =
        ConfigTestFixture::with_format(TerminalKind::Repl, OutputFormat::Json);
    let result = run_config_set(explain_fixture.command(), explain_args)
        .expect("session config set explain should succeed");
    assert!(matches!(result.output, Some(ReplCommandOutput::Json(_))));
}

#[test]
fn run_config_set_and_unset_reject_derived_profile_active_unit() {
    let mut set_args = config_set_args("profile.active", "ops");
    set_args.scope.global = true;
    let mut set_fixture = ConfigTestFixture::new(TerminalKind::Cli);
    let set_err = run_config_set(set_fixture.command(), set_args)
        .expect_err("profile.active set should be rejected");
    assert!(set_err.to_string().contains("read-only"));

    let mut unset_args = config_unset_args("profile.active");
    unset_args.scope.global = true;
    let mut unset_fixture = ConfigTestFixture::new(TerminalKind::Cli);
    let unset_err = run_config_unset(unset_fixture.command(), unset_args)
        .expect_err("profile.active unset should be rejected");
    assert!(unset_err.to_string().contains("read-only"));
}

#[test]
fn run_config_set_rejects_non_positive_operational_limit_unit() {
    let mut fixture = ConfigTestFixture::new(TerminalKind::Repl);

    let error = run_config_set(fixture.command(), config_set_args("ui.width", "0"))
        .expect_err("config set must reject a non-positive render width");

    assert!(
        error.to_string().contains("invalid value for key"),
        "unexpected config set error: {error:?}"
    );
    assert!(fixture.config_overrides.entries().is_empty());
}

#[test]
fn persistent_config_writes_are_rejected_when_file_loading_is_disabled_unit() {
    let mut set_args = config_set_args("ui.format", "json");
    set_args.store.config_store = true;
    let mut set_fixture = ConfigTestFixture::new(TerminalKind::Cli);
    let set_err = run_config_set(set_fixture.command(), set_args)
        .expect_err("a sealed session must not discover an ambient write path");
    assert!(
        set_err
            .to_string()
            .contains("config file writes are disabled for this session")
    );

    let mut unset_args = config_unset_args("ui.format");
    unset_args.store.secrets = true;
    let mut unset_fixture = ConfigTestFixture::new(TerminalKind::Cli);
    let unset_err = run_config_unset(unset_fixture.command(), unset_args)
        .expect_err("a sealed session must not discover an ambient secrets path");
    assert!(
        unset_err
            .to_string()
            .contains("config file writes are disabled for this session")
    );
}

#[cfg_attr(miri, ignore = "persistent config filesystem integration test")]
#[test]
fn run_config_set_and_unset_cover_persistent_paths_and_warning_unit() {
    with_temp_config_paths(|config_path, secrets_path| {
        let mut config_args = config_set_args("ui.format", "json");
        config_args.store.config_store = true;
        let mut config_fixture = ConfigTestFixture::new(TerminalKind::Cli);
        config_fixture.runtime_load = RuntimeLoadOptions::default();
        let config_set = run_config_set(config_fixture.command(), config_args)
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
        secrets_args.store.secrets = true;
        let mut secrets_fixture = ConfigTestFixture::new(TerminalKind::Cli);
        secrets_fixture.runtime_load = RuntimeLoadOptions::default();
        let secrets_set = run_config_set(secrets_fixture.command(), secrets_args)
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
        unset_args.store.secrets = true;
        let mut unset_fixture = ConfigTestFixture::new(TerminalKind::Cli);
        unset_fixture.runtime_load = RuntimeLoadOptions::default();
        let secrets_unset = run_config_unset(unset_fixture.command(), unset_args)
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
        missing_args.store.config_store = true;
        let mut missing_fixture = ConfigTestFixture::new(TerminalKind::Cli);
        missing_fixture.runtime_load = RuntimeLoadOptions::default();
        let missing_unset = run_config_unset(missing_fixture.command(), missing_args)
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
#[cfg_attr(miri, ignore = "config permission filesystem integration test")]
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

#[cfg_attr(miri, ignore = "config diagnostics inspects host-backed secrets paths")]
#[test]
fn config_diagnostics_rows_include_secrets_status_unit() {
    let rows = config_diagnostics_rows(ConfigTestFixture::new(TerminalKind::Cli).read());
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains_key("secrets_backend"));
    assert!(rows[0].contains_key("secrets_index_file"));
    assert!(rows[0].contains_key("secrets_permissions_status"));
    assert!(rows[0].contains_key("theme_issue_count"));
}

#[test]
fn secrets_backend_selector_cannot_be_written_to_the_store_it_selects_unit() {
    assert!(validate_store_key(ConfigStore::Config, "secrets.backend").is_ok());
    assert!(validate_store_key(ConfigStore::Secrets, "extensions.demo.token").is_ok());
    assert!(validate_store_key(ConfigStore::Secrets, "secrets.backend").is_err());
}
