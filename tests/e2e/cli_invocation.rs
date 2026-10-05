#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{
    ReplPtyConfig, ReplPtySession, first_json_row, osp_command, parse_json_stdout,
    strip_ansi_preserve_newlines, write_config,
};
#[cfg(unix)]
use crate::temp_support::make_temp_dir;

#[cfg(unix)]
#[test]
fn process_level_profile_selection_affects_builtin_command_output() {
    let home = make_temp_dir("osp-e2e-cli-profile-home");
    write_config(
        home.path(),
        r#"
[default]
profile.default = "uio"

[profile.uio]
theme.name = "nord"

[profile.tsd]
theme.name = "dracula"
"#,
    );

    let default_output = osp_command(home.path())
        .args(["--json", "config", "get", "theme.name"])
        .assert()
        .success()
        .get_output()
        .clone();
    let default_payload = parse_json_stdout(&default_output.stdout);
    let default_row = first_json_row(&default_payload, "default profile config get");
    assert_eq!(default_row["value"], "nord");

    let selected_output = osp_command(home.path())
        .args(["--json", "--profile", "tsd", "config", "get", "theme.name"])
        .assert()
        .success()
        .get_output()
        .clone();
    let selected_payload = parse_json_stdout(&selected_output.stdout);
    let selected_row = first_json_row(&selected_payload, "selected profile config get");
    assert_eq!(selected_row["value"], "dracula");
    assert!(
        selected_output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&selected_output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn repl_history_capture_and_maintenance_survive_processes_with_profile_scope() {
    let home = make_temp_dir("osp-e2e-history-lifecycle-home");
    let history = home.path().join("operator-history.jsonl");
    write_config(
        home.path(),
        r#"
[default]
profile.default = "uio"
repl.intro = "none"
repl.simple_prompt = true

[profile.uio]
theme.name = "nord"

[profile.tsd]
theme.name = "dracula"
"#,
    );
    let capture = |profile: &str, input: &str| {
        let config = format!(
            "[default]\nprofile.default = {profile:?}\nrepl.history.path = {:?}\n",
            history.to_str().unwrap()
        );
        let mut session = ReplPtySession::spawn(ReplPtyConfig::default().with_config(&config));
        let timeout = std::time::Duration::from_secs(3);
        let prompt = format!("{profile}>");
        assert!(session.wait_for_plain_output(&prompt, timeout));
        session.type_text("config set --session repl.history.enabled true");
        let start = session.output_len();
        session.write_bytes(b"\r");
        assert!(session.wait_for_plain_output_since(start, "for this session only", timeout));
        assert!(session.wait_for_plain_output_since(start, &prompt, timeout));
        let mut results = Vec::new();
        for command in input.lines().filter(|command| *command != "exit") {
            session.type_text(command);
            let start = session.output_len();
            session.write_bytes(b"\r");
            assert!(
                session.wait_for_plain_output_since(start, "} ]", timeout),
                "{}",
                session.output_snapshot(8000)
            );
            let output = strip_ansi_preserve_newlines(&session.output_since(start));
            let json_start = output
                .find("[\n")
                .expect("executed REPL command should return JSON");
            results.push(
                serde_json::Deserializer::from_str(&output[json_start..])
                    .into_iter::<serde_json::Value>()
                    .next()
                    .unwrap()
                    .unwrap(),
            );
            assert!(session.wait_for_plain_output_since(start, &prompt, timeout));
        }
        session.write_bytes(b"exit\r");
        assert!(
            session.wait_for_exit(timeout),
            "{}",
            session.output_snapshot(8000)
        );
        results
    };
    let uio = capture(
        "uio",
        "theme show nord --json\ntheme show dracula --json\nexit\n",
    );
    assert_eq!(uio[0][0]["id"], "nord");
    assert_eq!(uio[1][0]["id"], "dracula");
    let tsd = capture(
        "tsd",
        "theme show rose-pine-moon --json\ntheme show nord --json\nexit\n",
    );
    assert_eq!(tsd[0][0]["id"], "rose-pine-moon");
    assert_eq!(tsd[1][0]["id"], "nord");

    let list = |profile: &str| {
        let output = osp_command(home.path())
            .timeout(std::time::Duration::from_secs(5))
            .env("OSP__REPL__HISTORY__PATH", &history)
            .args(["--profile", profile, "--json", "history", "list"])
            .assert()
            .success()
            .get_output()
            .clone();
        let entries = parse_json_stdout(&output.stdout);
        for entry in entries.as_array().unwrap() {
            assert!(entry["timestamp_ms"].as_i64().unwrap() > 0);
        }
        entries
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["command"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        list("uio"),
        ["theme show nord --json", "theme show dracula --json"]
    );
    assert_eq!(
        list("tsd"),
        ["theme show rose-pine-moon --json", "theme show nord --json"]
    );
    let pruned = osp_command(home.path())
        .timeout(std::time::Duration::from_secs(5))
        .env("OSP__REPL__HISTORY__PATH", &history)
        .args(["--profile", "uio", "history", "prune", "1"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        String::from_utf8(pruned.stdout).unwrap(),
        "Removed 1 entry from root history.\n"
    );
    assert_eq!(list("uio"), ["theme show dracula --json"]);
    assert_eq!(
        list("tsd"),
        ["theme show rose-pine-moon --json", "theme show nord --json"]
    );
    let cleared = osp_command(home.path())
        .timeout(std::time::Duration::from_secs(5))
        .env("OSP__REPL__HISTORY__PATH", &history)
        .args(["--profile", "tsd", "history", "clear"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        String::from_utf8(cleared.stdout).unwrap(),
        "Cleared root history.\n"
    );
    let resumed = capture("tsd", "theme show dracula --json\nexit\n");
    assert_eq!(resumed[0][0]["id"], "dracula");
    assert_eq!(list("tsd"), ["theme show dracula --json"]);
    assert_eq!(list("uio"), ["theme show dracula --json"]);
}

#[cfg(unix)]
#[test]
fn explicit_config_path_env_is_respected_by_real_binary() {
    let home = make_temp_dir("osp-e2e-cli-config-path-home");
    write_config(
        home.path(),
        r#"
[default]
profile.default = "uio"
theme.name = "nord"
"#,
    );

    let explicit_config = home.path().join("explicit.toml");
    std::fs::write(
        &explicit_config,
        r#"
[default]
profile.default = "uio"
theme.name = "dracula"
"#,
    )
    .expect("explicit config should be written");

    let output = osp_command(home.path())
        .env("OSP_CONFIG_FILE", &explicit_config)
        .args(["--json", "config", "get", "theme.name"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "explicit config path env");
    assert_eq!(row["value"], "dracula");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn closed_stdout_pipe_is_a_normal_process_exit() {
    use std::process::{Command, Stdio};

    let home = make_temp_dir("osp-e2e-cli-closed-pipe-home");
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("osp"))
        .env_clear()
        .envs(crate::test_env::isolated_env(home.path()))
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "dumb")
        .env("LANG", "C.UTF-8")
        .arg("--help")
        .stdout(Stdio::piped())
        .spawn()
        .expect("OSP binary should start");
    drop(child.stdout.take());

    let status = child.wait().expect("OSP binary should exit");
    assert!(
        status.success(),
        "broken stdout pipe should not panic: {status}"
    );
}
