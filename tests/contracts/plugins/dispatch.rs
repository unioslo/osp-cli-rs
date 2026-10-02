#[cfg(unix)]
#[test]
fn plugin_sdk_metadata_flows_through_host_catalog_policy_and_dispatch_contract() {
    use clap::{Arg, ArgAction, builder::PossibleValue};
    use osp_cli::app::BufferedUiSink;
    use osp_cli::completion::SuggestionEntry;
    use osp_cli::core::command_def::{CommandDef, CommandPolicyDef};
    use osp_cli::core::command_policy::{
        AuthStrength, CommandAccess, CommandPath, CommandPolicyContext, CredentialRequirement,
        CredentialState, SessionRequirements, VisibilityMode,
    };
    use osp_cli::core::plugin::{DescribeCommandV1, DescribeV1, PLUGIN_PROTOCOL_V1};
    use osp_cli::plugin::{PluginDispatchContext, PluginManager, PluginSource};

    let policy = CommandPolicyDef {
        visibility: VisibilityMode::Authenticated,
        required_capabilities: vec!["directory.read".to_string()],
        feature_flags: vec!["directory".to_string()],
        visible_session_requirements: SessionRequirements {
            auth_strength: Some(AuthStrength::Basic),
            credentials: vec![CredentialRequirement::present("audit")],
        },
        run_session_requirements: SessionRequirements {
            auth_strength: Some(AuthStrength::Basic),
            credentials: vec![
                CredentialRequirement::valid("directory"),
                CredentialRequirement::fresh("osp", 300),
            ],
        },
    };
    let command = CommandDef::from_clap(
        clap::Command::new("sdk-directory")
            .about("Inspect directory records")
            .subcommand(
                clap::Command::new("inspect")
                    .about("Read selected record collections")
                    .arg(
                        Arg::new("collection")
                            .value_name("COLLECTION")
                            .help("Collections to inspect")
                            .required(true)
                            .num_args(1..)
                            .value_parser([
                                PossibleValue::new("people").help("Person records"),
                                PossibleValue::new("groups").help("Group records"),
                            ]),
                    )
                    .arg(
                        Arg::new("format")
                            .long("format")
                            .short('f')
                            .visible_alias("fmt")
                            .visible_short_alias('o')
                            .help("Export formats")
                            .required(true)
                            .action(ArgAction::Append)
                            .value_parser([
                                PossibleValue::new("json").help("Typed JSON"),
                                PossibleValue::new("table").help("Readable table"),
                            ]),
                    ),
            ),
    )
    .policy(policy.clone());
    let describe = DescribeV1 {
        protocol_version: PLUGIN_PROTOCOL_V1,
        plugin_id: "sdk".to_string(),
        plugin_version: "0.1.0".to_string(),
        min_osp_version: Some("0.1.0".to_string()),
        commands: vec![DescribeCommandV1::from(&command)],
    };
    describe.validate_v1().expect("SDK metadata should be valid");
    let wire = serde_json::to_value(&describe).expect("metadata should serialize");
    assert_eq!(
        wire["commands"][0]["auth"],
        serde_json::json!({
            "visibility": "authenticated",
            "required_capabilities": ["directory.read"],
            "feature_flags": ["directory"],
            "visible_session": {
                "auth_strength": "basic",
                "credentials": [{"state": "present", "service": "audit"}]
            },
            "run_session": {
                "auth_strength": "basic",
                "credentials": [
                    {"state": "valid", "service": "directory"},
                    {"state": "fresh", "service": "osp", "min_ttl_seconds": 300}
                ]
            }
        })
    );

    let dir = make_temp_dir("osp-cli-plugin-sdk-consumer");
    let plugin_path = write_provider_plugin(&dir, "sdk", "sdk-directory", "sdk");
    let fixture = std::fs::read_to_string(&plugin_path).expect("fixture should be readable");
    let (prefix, describe_and_execution) = fixture
        .split_once("cat <<'JSON'\n")
        .expect("fixture should publish describe JSON");
    let (_, execution) = describe_and_execution
        .split_once("\nJSON")
        .expect("fixture should terminate describe JSON");
    std::fs::write(
        &plugin_path,
        format!("{prefix}cat <<'JSON'\n{wire}\nJSON{execution}"),
    )
    .expect("SDK metadata should be installed in the executable fixture");

    let manager = PluginManager::new(vec![dir.path().to_path_buf()])
        .with_default_roots(false)
        .with_bundled_roots(false)
        .with_path_discovery(false);
    let catalog = manager.command_catalog();
    assert_eq!(
        catalog.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(),
        vec!["sdk-directory"]
    );
    let entry = &catalog[0];
    assert_eq!(entry.about, "Inspect directory records");
    assert_eq!(entry.provider.as_deref(), Some("sdk"));
    assert_eq!(entry.providers, vec!["sdk (explicit)".to_string()]);
    assert_eq!(entry.source, Some(PluginSource::Explicit));
    assert_eq!(entry.subcommands, vec!["inspect".to_string()]);
    assert_eq!(serde_json::to_value(&entry.auth).unwrap(), wire["commands"][0]["auth"]);
    let inspect = &entry.completion.subcommands[0];
    assert_eq!(inspect.name, "inspect");
    assert_eq!(inspect.tooltip.as_deref(), Some("Read selected record collections"));
    assert_eq!(inspect.args[0].name.as_deref(), Some("COLLECTION"));
    assert_eq!(inspect.args[0].tooltip.as_deref(), Some("Collections to inspect"));
    assert!(inspect.args[0].required && inspect.args[0].multi);
    assert_eq!(
        inspect.args[0].suggestions,
        vec![
            SuggestionEntry::value("people").meta("Person records"),
            SuggestionEntry::value("groups").meta("Group records"),
        ]
    );
    assert_eq!(
        inspect.flags.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["--fmt", "--format", "-f", "-o"]
    );
    let format = &inspect.flags["--format"];
    assert_eq!(format.tooltip.as_deref(), Some("Export formats"));
    assert!(format.multi);
    assert_eq!(
        format.suggestions,
        vec![
            SuggestionEntry::value("json").meta("Typed JSON"),
            SuggestionEntry::value("table").meta("Readable table"),
        ]
    );
    // The wire lists spellings without a preferred one; the host keeps the
    // first long spelling and marks the rest as its aliases.
    assert_eq!(inspect.flags["--fmt"].alias_of, None);
    for spelling in ["--fmt", "--format", "-f", "-o"] {
        let node = &inspect.flags[spelling];
        assert_eq!(
            &osp_cli::completion::FlagNode {
                alias_of: format.alias_of.clone(),
                ..node.clone()
            },
            format
        );
    }
    for spelling in ["--format", "-f", "-o"] {
        assert_eq!(inspect.flags[spelling].alias_of.as_deref(), Some("--fmt"));
    }

    let context = CommandPolicyContext::default()
        .with_auth_strength(AuthStrength::Basic)
        .with_capabilities(["directory.read"])
        .with_features(["directory"])
        .with_credential("audit", CredentialState::present())
        .with_credential("directory", CredentialState::valid())
        .with_credential("osp", CredentialState::valid_for(900));
    let registry = manager.command_policy_registry();
    for path in [
        CommandPath::new(["sdk-directory"]),
        CommandPath::new(["sdk-directory", "inspect"]),
    ] {
        let resolved = registry.resolved_policy(&path).expect("policy should be registered");
        assert_eq!(resolved.visibility, policy.visibility);
        assert_eq!(
            resolved.required_capabilities.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["directory.read"]
        );
        assert_eq!(
            resolved.feature_flags.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["directory"]
        );
        assert_eq!(resolved.visible_session_requirements, policy.visible_session_requirements);
        assert_eq!(resolved.run_session_requirements, policy.run_session_requirements);
        assert_eq!(registry.evaluate(&path, &context), Some(CommandAccess::visible_runnable()));
    }
    let response = manager
        .dispatch(
            "sdk-directory",
            &["inspect".to_string(), "people".to_string(), "--fmt".to_string(), "json".to_string()],
            &PluginDispatchContext::default(),
        )
        .expect("described provider should execute");
    assert_eq!(response.protocol_version, PLUGIN_PROTOCOL_V1);
    assert!(response.ok);
    assert_eq!(response.data, serde_json::json!({"message": "sdk-from-plugin"}));
    assert_eq!(response.meta.format_hint.as_deref(), Some("table"));
    assert_eq!(response.meta.columns, Some(vec!["message".to_string()]));

    let mut sink = BufferedUiSink::default();
    let exit = osp_cli::App::new()
        .with_policy_context(context)
        .run_with_sink(
            ["osp", "--defaults-only", "--plugin-dir", dir.to_str().unwrap(), "--json",
                "sdk-directory", "inspect", "people", "--fmt", "json"],
            &mut sink,
        )
        .expect("authenticated embedding host should execute the inherited command policy");
    assert_eq!(exit, 0);
    assert_eq!(parse_json_stdout(sink.stdout.as_bytes()), serde_json::json!([{"message": "sdk-from-plugin"}]));
    assert!(sink.stderr.is_empty(), "unexpected host diagnostics: {}", sink.stderr);
}

#[cfg(unix)]
#[test]
fn external_plugin_dispatch_contract() {
    let dir = make_temp_dir("osp-cli-plugin-exec");
    let _plugin_path = write_hello_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-exec-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["hello"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("hello-from-plugin"));

}

#[test]
fn plugin_dispatch_propagates_runtime_hints_contract() {
    let dir = make_temp_dir("osp-cli-plugin-runtime-hints");
    let _plugin_path = write_hints_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-runtime-home");
    write_config(&home, "[profile.tsd]\n");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .env("TERM", "xterm-256color")
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args([
            "--profile",
            "tsd",
            "-vv",
            "-ddd",
            "--json",
            "--color",
            "never",
            "--unicode",
            "always",
            "hints",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "plugin runtime hints");
    assert_eq!(row["ui_verbosity"], "trace");
    assert_eq!(row["debug_level"], "3");
    assert_eq!(row["format"], "json");
    assert_eq!(row["color"], "never");
    assert_eq!(row["unicode"], "always");
    assert_eq!(row["profile"], "tsd");
    assert_eq!(row["terminal_kind"], "cli");
    assert_eq!(row["terminal"], "xterm-256color");

}

#[cfg(unix)]
#[test]
fn plugin_dispatch_propagates_config_env_contract() {
    let dir = make_temp_dir("osp-cli-plugin-config-env");
    let _plugin_path = write_config_env_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-config-env-home");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
extensions.plugins.env.shared.url = "https://common.example"
extensions.plugins.env.endpoint = "shared-endpoint"
extensions.plugins.cfg.env.endpoint = "plugin-endpoint"
extensions.plugins.cfg.env.api.token = "token-123"
extensions.plugins.cfg.env.enable_cache = true
extensions.plugins.cfg.env.retries = 3
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "cfg"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "plugin config env");
    assert_eq!(row["shared_url"], "https://common.example");
    assert_eq!(row["endpoint"], "plugin-endpoint");
    assert_eq!(row["api_token"], "token-123");
    assert_eq!(row["enable_cache"], "true");
    assert_eq!(row["retries"], "3");
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn plugin_dispatch_uses_an_explicit_environment_boundary_contract() {
    let dir = make_temp_dir("osp-cli-plugin-env-boundary");
    let _plugin_path = write_env_boundary_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-env-boundary-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/tmp/parent-secret-path")
        .env("OSP_PASSWORD", "synthetic-password")
        .env("OSP_MFA_TOKEN", "synthetic-mfa")
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "env-boundary"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "plugin environment boundary");
    assert_eq!(row["password"], "");
    assert_eq!(row["mfa"], "");
    assert_eq!(row["path"], "/tmp/parent-secret-path");
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn plugin_non_zero_exit_surfaces_stderr_contract() {
    let dir = make_temp_dir("osp-cli-plugin-non-zero-exit");
    let _plugin_path = write_non_zero_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-non-zero-exit-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["boom"]);
    let output = cmd.assert().failure().get_output().clone();
    assert!(
        output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_snapshot_text!(
        "plugin_non_zero_exit_stderr",
        String::from_utf8(output.stderr).expect("stderr should be utf-8"),
    );

}

#[cfg(unix)]
#[test]
fn plugin_invalid_json_response_surfaces_contract() {
    let dir = make_temp_dir("osp-cli-plugin-invalid-json");
    let _plugin_path = write_invalid_json_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-invalid-json-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["broken"]);
    cmd.assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "invalid JSON response from plugin broken",
        ));

}

#[cfg(unix)]
#[test]
fn plugin_messages_stay_on_stderr_when_data_is_json_contract() {
    let dir = make_temp_dir("osp-cli-plugin-messages-json");
    let _plugin_path = write_message_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-messages-json-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "messageful"]);
    let output = cmd.assert().success().get_output().clone();
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    let payload = parse_json_stdout(stdout.as_bytes());
    let row = first_json_row(&payload, "plugin json message output");
    assert_eq!(row["message"], "json-from-plugin");
    assert!(!stdout.contains("plugin-warning-line"));
    assert_snapshot_text!("plugin_messages_json_stdout", stdout);
    assert_snapshot_text!("plugin_messages_json_stderr", stderr);

}

#[cfg(unix)]
#[test]
fn plugins_config_reports_projected_env_contract() {
    let home = make_temp_dir("osp-cli-plugin-config-view-home");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
extensions.plugins.env.endpoint = "shared-endpoint"
extensions.plugins.env.shared.url = "https://common.example"
extensions.plugins.cfg.env.endpoint = "plugin-endpoint"
extensions.plugins.cfg.env.retries = 3
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .args(["--json", "plugins", "config", "cfg"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let rows = payload
        .as_array()
        .expect("plugins config should render a JSON array");
    assert!(
        rows.iter().any(|row| {
            row["plugin_id"] == "cfg"
                && row["env"] == "OSP_PLUGIN_CFG_ENDPOINT"
                && row["value"] == "plugin-endpoint"
                && row["config_key"] == "extensions.plugins.cfg.env.endpoint"
                && row["scope"] == "plugin"
        }),
        "expected plugin-scoped endpoint row in payload: {payload}"
    );
    assert!(
        rows.iter().any(|row| {
            row["plugin_id"] == "cfg"
                && row["env"] == "OSP_PLUGIN_CFG_SHARED_URL"
                && row["value"] == "https://common.example"
                && row["config_key"] == "extensions.plugins.env.shared.url"
                && row["scope"] == "shared"
        }),
        "expected shared env row in payload: {payload}"
    );
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn multi_command_plugin_receives_selected_command_contract() {
    let dir = make_temp_dir("osp-cli-plugin-multi-command");
    let _plugin_path = write_multi_command_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-multi-command-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "alpha", "run"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "multi-command plugin dispatch");
    assert_eq!(row["selected_command"], "alpha");
    assert_eq!(row["arg0"], "alpha");
    assert_eq!(row["arg1"], "run");
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut batch = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let batch_output = batch
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args([
            "--json", "beta", "run", "alice", "bob", "carol", "--label", "batch",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let batch_payload = parse_json_stdout(&batch_output.stdout);
    assert_eq!(
        first_json_row(&batch_payload, "nested variadic plugin dispatch"),
        &serde_json::json!({
            "selected_command": "beta",
            "arg0": "beta",
            "arg1": "run",
            "arg2": "alice",
            "arg3": "bob",
            "arg4": "carol",
            "arg5": "--label",
            "arg6": "batch",
        })
    );
    assert!(batch_output.stderr.is_empty());

}

#[cfg(unix)]
#[test]
fn oneshot_dispatch_does_not_use_repl_session_cache_contract() {
    let dir = make_temp_dir("osp-cli-plugin-counter");
    let _plugin_path = write_counter_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-counter-home");

    let mut first = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    first
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--unicode", "never", "counter"]);
    first
        .assert()
        .success()
        .stdout(predicate::str::contains("| 1"));

    let mut second = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    second
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--unicode", "never", "counter"]);
    second
        .assert()
        .success()
        .stdout(predicate::str::contains("| 2"));

}

#[cfg(unix)]
#[test]
fn describe_cache_is_reused_and_invalidated_contract() {
    let dir = make_temp_dir("osp-cli-plugin-describe-cache");
    let plugin_path = write_describe_counter_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-describe-cache-home");
    let describe_count_path = dir.join("describe-count.txt");

    let mut first = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    first
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    let first_output = first
        .assert()
        .success()
        .get_output()
        .clone();
    let first_payload = parse_json_stdout(&first_output.stdout);
    assert!(
        first_payload
            .as_array()
            .expect("plugins list should render a JSON array")
            .iter()
            .any(|row| row["plugin_version"] == "0.1.1"),
        "expected plugin_version 0.1.1 in payload: {first_payload}"
    );
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("describe count should be written")
            .trim(),
        "1"
    );

    let mut second = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    second
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    let second_output = second
        .assert()
        .success()
        .get_output()
        .clone();
    let second_payload = parse_json_stdout(&second_output.stdout);
    assert!(
        second_payload
            .as_array()
            .expect("plugins list should render a JSON array")
            .iter()
            .any(|row| row["plugin_version"] == "0.1.1"),
        "expected cached plugin_version 0.1.1 in payload: {second_payload}"
    );
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("describe count should still be readable")
            .trim(),
        "1"
    );

    let mut script =
        std::fs::read_to_string(&plugin_path).expect("plugin script should be readable");
    script = script
        .replace("describe counter plugin", "upgraded describe counter plugin")
        .replace("\"message\":\"ok\"", "\"message\":\"upgraded\"");
    std::fs::write(&plugin_path, script).expect("plugin script should be updated");

    let mut third = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    third
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    let third_output = third
        .assert()
        .success()
        .get_output()
        .clone();
    let third_payload = parse_json_stdout(&third_output.stdout);
    assert!(
        third_payload
            .as_array()
            .expect("plugins list should render a JSON array")
            .iter()
            .any(|row| row["plugin_version"] == "0.1.2"),
        "expected invalidated plugin_version 0.1.2 in payload: {third_payload}"
    );
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("describe count should reflect invalidation")
            .trim(),
        "2"
    );

    let mut catalog = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let catalog_output = catalog
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "commands"])
        .assert()
        .success()
        .get_output()
        .clone();
    let catalog_payload = parse_json_stdout(&catalog_output.stdout);
    let command = first_json_row(&catalog_payload, "upgraded plugin catalog");
    assert_eq!(command["name"], "describe-counter");
    assert_eq!(command["provider"], "describe-counter");
    assert_eq!(command["about"], "upgraded describe counter plugin");
    assert_eq!(command["source"], "env");

    let mut execute = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let execution_output = execute
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "describe-counter"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        first_json_row(
            &parse_json_stdout(&execution_output.stdout),
            "upgraded plugin execution",
        ),
        &serde_json::json!({ "message": "upgraded" })
    );
    assert!(execution_output.stderr.is_empty());
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("upgraded metadata should stay cached across catalog and execution")
            .trim(),
        "2"
    );
    // A cache is an optimization: an interrupted write or unavailable cache
    // location still permits fresh metadata and useful command execution.
    let cache_path = home.join(".cache/osp/describe-v1.json");
    let run = |args: &[&str]| {
        Command::new(assert_cmd::cargo::cargo_bin!("osp"))
            .envs(crate::test_env::isolated_env(&home))
            .env("OSP_PLUGIN_PATH", &dir)
            .args(args)
            .assert()
            .success()
            .get_output()
            .clone()
    };
    let describe_count = || {
        std::fs::read_to_string(&describe_count_path)
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap()
    };
    std::fs::write(&cache_path, "{interrupted cache write").unwrap();
    let repaired = run(&["-dd", "--json", "plugins", "list"]);
    let repaired_plugins = parse_json_stdout(&repaired.stdout);
    let repaired_plugin = first_json_row(&repaired_plugins, "repaired describe cache");
    assert_eq!(repaired_plugin["plugin_id"], "describe-counter");
    assert_eq!(repaired_plugin["plugin_version"], "0.1.3");
    assert_eq!(describe_count(), 3);
    let diagnostics = String::from_utf8(repaired.stderr).unwrap();
    assert!(diagnostics.contains("WARN"));
    assert!(diagnostics.contains(cache_path.to_str().unwrap()));
    let reused = run(&["--json", "describe-counter"]);
    assert_eq!(
        first_json_row(
            &parse_json_stdout(&reused.stdout),
            "repaired cache execution"
        ),
        &serde_json::json!({"message": "upgraded"})
    );
    assert_eq!(describe_count(), 3);

    std::fs::remove_file(&cache_path).unwrap();
    std::fs::create_dir(&cache_path).unwrap();
    let uncached = run(&["-dd", "--json", "describe-counter"]);
    assert_eq!(
        first_json_row(&parse_json_stdout(&uncached.stdout), "uncached execution"),
        &serde_json::json!({"message": "upgraded"})
    );
    assert!(describe_count() > 3);
    let diagnostics = String::from_utf8(uncached.stderr).unwrap();
    assert!(diagnostics.contains("WARN"));
    assert!(diagnostics.contains(cache_path.to_str().unwrap()));

    std::fs::remove_dir(&cache_path).unwrap();
    let restored = run(&["--json", "plugins", "commands"]);
    let restored_commands = parse_json_stdout(&restored.stdout);
    assert_eq!(
        first_json_row(&restored_commands, "restored cache catalogue")["about"],
        "upgraded describe counter plugin"
    );
    let restored_count = describe_count();
    run(&["--json", "describe-counter"]);
    assert_eq!(describe_count(), restored_count);
}
