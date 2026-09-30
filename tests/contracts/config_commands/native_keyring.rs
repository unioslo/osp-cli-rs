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
    // The index and native credential must agree after each transaction,
    // including when the operator must restore index write access.
    let binary = assert_cmd::cargo::cargo_bin!("osp")
        .to_str()
        .unwrap()
        .to_owned();
    let mut operations = commands
        .as_array()
        .unwrap()
        .iter()
        .map(|args| {
            let mut argv = vec![serde_json::Value::String(binary.to_string())];
            argv.extend(args.as_array().unwrap().iter().cloned());
            serde_json::Value::Array(argv)
        })
        .collect::<Vec<_>>();
    let scoped_set = |value| {
        serde_json::json!([
            binary,
            "--json",
            "config",
            "set",
            key,
            value,
            "--secrets",
            "--profile",
            "tsd",
            "--terminal",
            "cli"
        ])
    };
    let scoped_unset = serde_json::json!([
        binary,
        "--json",
        "config",
        "unset",
        key,
        "--secrets",
        "--profile",
        "tsd",
        "--terminal",
        "cli"
    ]);
    let read = serde_json::json!([binary, "--json", "config", "explain", key, "--show-secrets"]);
    let index_parent = home.join(".config/osp");
    operations.extend([
        scoped_set("throwaway-token-v3"),
        read.clone(),
        serde_json::json!(["chmod", "0500", index_parent]),
        scoped_set("throwaway-token-v4"),
        read.clone(),
        scoped_unset.clone(),
        read.clone(),
        serde_json::json!(["chmod", "u+w", index_parent]),
        scoped_set("throwaway-token-v4"),
        read.clone(),
        scoped_unset,
        read,
    ]);
    let output = Command::new("python3")
        .env_clear()
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/native_keyring.py"
        ))
        .arg(home.as_os_str())
        .arg(serde_json::to_string(&operations).unwrap())
        .timeout(std::time::Duration::from_secs(135))
        .assert()
        .success()
        .get_output()
        .clone();
    let results: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
        .expect("native fixture should return captured CLI operations");
    for operation in &results[..12] {
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

    for operation in &results[12..] {
        assert_eq!(operation["mode"], 0o600, "{operation}");
        assert_eq!(operation["native_store"], true, "{operation}");
    }
    for operation in [12, 13, 14, 16, 18, 19, 20, 21, 22, 23] {
        assert_eq!(results[operation]["exit_code"], 0, "{}", results[operation]);
    }
    for operation in [15, 17] {
        assert_ne!(results[operation]["exit_code"], 0, "{}", results[operation]);
    }
    assert_eq!(results[14]["index_parent_mode"], 0o500);
    assert_eq!(results[19]["index_parent_mode"], 0o700);
    for operation in &results[12..22] {
        let index: toml::Value = toml::from_str(operation["index"].as_str().unwrap()).unwrap();
        assert_eq!(
            index, expected_index,
            "scoped identity survives operator recovery"
        );
    }
    for (operation, value) in [
        (13, "throwaway-token-v3"),
        (16, "throwaway-token-v3"),
        (18, "throwaway-token-v3"),
        (21, "throwaway-token-v4"),
    ] {
        let credential = payload(operation);
        assert_eq!(credential["value"], value);
        assert_eq!(credential["source"], "secrets");
        assert_eq!(credential["scope"], "profile:tsd terminal:cli");
        assert_eq!(credential["origin"], "keyring:osp-cli:secrets:v1");
    }
    for operation in [20, 22] {
        let mutation = row(operation);
        assert_eq!(mutation["previous"], "[REDACTED]");
        assert_eq!(mutation["changed"], true);
        assert_eq!(mutation["backend"], "keyring");
    }
    let final_index: toml::Value = toml::from_str(results[22]["index"].as_str().unwrap()).unwrap();
    assert_eq!(final_index, cleared);
    let recovered_fallback = payload(23);
    assert_eq!(recovered_fallback["value"], "fallback-v0");
    assert_eq!(recovered_fallback["source"], "file");
    assert_eq!(recovered_fallback["scope"], "global");
}
