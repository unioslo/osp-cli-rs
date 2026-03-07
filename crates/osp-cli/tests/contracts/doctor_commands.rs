use assert_cmd::Command;

#[cfg(unix)]
#[test]
fn doctor_json_stdout_is_machine_parseable_contract() {
    let home = make_temp_dir("osp-cli-doctor-json");
    let empty_plugins = make_temp_dir("osp-cli-empty-plugins-doctor");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("OSP_PLUGIN_PATH", &empty_plugins)
        .env("OSP_BUNDLED_PLUGIN_DIR", &empty_plugins)
        .args(["--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let object = payload
        .as_object()
        .expect("doctor payload should be a json object");
    assert!(object.contains_key("config"));
    assert!(object.contains_key("plugins"));
    assert!(object.contains_key("theme"));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&empty_plugins);
}

#[cfg(unix)]
#[test]
fn doctor_last_without_recorded_failure_is_human_text_contract() {
    let home = make_temp_dir("osp-cli-doctor-last");
    let empty_plugins = make_temp_dir("osp-cli-empty-plugins-doctor-last");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("OSP_PLUGIN_PATH", &empty_plugins)
        .env("OSP_BUNDLED_PLUGIN_DIR", &empty_plugins)
        .args(["doctor", "last"]);

    cmd.assert()
        .success()
        .stdout(predicates::str::contains(
            "No recorded REPL failure in this session.",
        ))
        .stderr(predicates::str::is_empty());

    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&empty_plugins);
}

#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

#[cfg(unix)]
fn parse_json_stdout(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be valid json: {err}\n{}",
            String::from_utf8_lossy(stdout)
        )
    })
}
