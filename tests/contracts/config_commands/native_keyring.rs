#[cfg(target_os = "linux")]
#[test]
fn native_keyring_credentials_round_trip_scoped_cli_edits_and_private_index() {
    let home = make_temp_dir("osp-cli-native-keyring-contract");
    write_config(
        &home,
        r#"
[default]
profile.default = "tsd"
secrets.backend = "keyring"
extensions.site.token = "fallback-v0"
"#,
    );
    let key = "extensions.site.token";
    let commands = serde_json::json!([
        [
            "--json",
            "config",
            "set",
            key,
            "throwaway-token-v1",
            "--secrets",
            "--profile",
            "tsd",
            "--terminal",
            "cli"
        ],
        ["--json", "config", "doctor"],
        ["--json", "config", "explain", key],
        ["--json", "config", "explain", key, "--show-secrets"],
        [
            "--json",
            "config",
            "set",
            key,
            "throwaway-token-v2",
            "--secrets",
            "--profile",
            "tsd",
            "--terminal",
            "cli",
            "--dry-run"
        ],
        ["--json", "config", "explain", key, "--show-secrets"],
        [
            "--json",
            "config",
            "set",
            key,
            "throwaway-token-v2",
            "--secrets",
            "--profile",
            "tsd",
            "--terminal",
            "cli"
        ],
        ["--json", "config", "explain", key, "--show-secrets"],
        [
            "--json",
            "config",
            "unset",
            key,
            "--secrets",
            "--profile",
            "tsd",
            "--terminal",
            "cli",
            "--dry-run"
        ],
        ["--json", "config", "explain", key, "--show-secrets"],
        [
            "--json",
            "config",
            "unset",
            key,
            "--secrets",
            "--profile",
            "tsd",
            "--terminal",
            "cli"
        ],
        ["--json", "config", "explain", key, "--show-secrets"]
    ]);
    let output = Command::new("python3")
        .env_clear()
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/native_keyring.py"
        ))
        .arg(assert_cmd::cargo::cargo_bin!("osp"))
        .arg(home.as_os_str())
        .arg(commands.to_string())
        .timeout(std::time::Duration::from_secs(135))
        .assert()
        .success()
        .get_output()
        .clone();
    let results: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
        .expect("native fixture should return captured CLI operations");
    for operation in &results {
        assert_eq!(operation["exit_code"], 0, "{operation}");
        assert_eq!(operation["mode"], 0o600, "{operation}");
        assert_eq!(
            operation["native_store"], true,
            "native keyring must persist on disk"
        );
    }
    let payload =
        |index: usize| parse_json_stdout(results[index]["stdout"].as_str().unwrap().as_bytes());
    let row = |index| first_json_row(&payload(index), "native keyring CLI operation").clone();
    let initial = row(0);
    assert_eq!(initial["backend"], "keyring");
    assert_eq!(initial["scope"], "profile:tsd terminal:cli");
    assert_eq!(initial["value"], "[REDACTED]");
    assert_eq!(initial["previous"], serde_json::Value::Null);
    assert_eq!(initial["changed"], true);
    let index_path = home.join(".config/osp/secrets.index.toml");
    assert_eq!(initial["path"], index_path.display().to_string());
    let expected_index: toml::Value = toml::from_str(
        r#"
version = 1
[[entries]]
key = "extensions.site.token"
profile = "tsd"
terminal = "cli"
"#,
    )
    .unwrap();
    for operation in &results[..10] {
        let index: toml::Value = toml::from_str(operation["index"].as_str().unwrap()).unwrap();
        assert_eq!(
            index, expected_index,
            "index contains only the normalized scoped identity"
        );
    }
    let diagnostics = row(1);
    assert_eq!(diagnostics["secrets_backend"], "keyring");
    assert_eq!(
        diagnostics["secrets_store_path"],
        index_path.display().to_string()
    );
    assert_eq!(diagnostics["secrets_permissions_status"], "ok");
    assert_eq!(diagnostics["secrets_permissions_mode"], "600");
    assert_eq!(payload(2)["value"], "[REDACTED]");
    for (operation, value) in [
        (3, "throwaway-token-v1"),
        (5, "throwaway-token-v1"),
        (7, "throwaway-token-v2"),
        (9, "throwaway-token-v2"),
    ] {
        let exposed = payload(operation);
        assert_eq!(exposed["value"], value);
        assert_eq!(exposed["source"], "secrets");
        assert_eq!(exposed["scope"], "profile:tsd terminal:cli");
        assert_eq!(exposed["origin"], "keyring:osp-cli:secrets:v1");
    }
    for (operation, preview) in [(4, true), (6, false), (8, true), (10, false)] {
        let mutation = row(operation);
        assert_eq!(mutation["previous"], "[REDACTED]");
        assert_eq!(mutation["changed"], true);
        assert_eq!(mutation["dry_run"], preview);
        assert_eq!(mutation["backend"], "keyring");
    }
    let cleared: toml::Value = toml::from_str(results[10]["index"].as_str().unwrap()).unwrap();
    assert_eq!(
        cleared,
        toml::from_str::<toml::Value>("version = 1\nentries = []\n").unwrap()
    );
    let fallback = payload(11);
    assert_eq!(fallback["value"], "fallback-v0");
    assert_eq!(fallback["source"], "file");
    assert_eq!(fallback["scope"], "global");
}
