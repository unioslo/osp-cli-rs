use crate::temp_support::make_temp_dir;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_basic_repl(input: &[u8]) -> Output {
    let home = make_temp_dir("osp-cli-repl-basic-home");

    Command::new(env!("CARGO_BIN_EXE_osp"))
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("XDG_CACHE_HOME", home.path().join(".cache"))
        .env("XDG_STATE_HOME", home.path().join(".local/state"))
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .arg("--defaults-only")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .as_mut()
                .expect("stdin should be piped")
                .write_all(input)?;
            child.wait_with_output()
        })
        .expect("basic repl should run")
}

#[test]
fn repl_basic_mode_runs_help_and_exit_without_tty() {
    let files = make_temp_dir("osp-cli-repl-source");
    let commands = files.path().join("session commands.osp");
    std::fs::write(
        &commands,
        "# Read config through the same session\n\nconfig get theme.name\n",
    )
    .expect("command file should write");
    let first = files.path().join("first result.osp");
    let second = files.path().join("second result.osp");
    std::fs::write(&first, "theme show dracula | P id,name\n").unwrap();
    std::fs::write(&second, "theme show nord | P id,name\n").unwrap();
    let recovery = files.path().join("recovery batch.osp");
    std::fs::write(
        &recovery,
        "theme show dracula | P id,name\ntheme show dracula | Z\ntheme show nord | P id,name\n",
    )
    .unwrap();
    let input = format!(
        "help\nsource '{}'\n!!\nconfig set --session ui.format json\nsource -- '{}' '{}'\nlast\nlast --raw\nsource '{}'\nlast\nsource --ignore-errors '{}'\nlast\nexit\n",
        commands.display(),
        first.display(),
        second.display(),
        recovery.display(),
        recovery.display(),
    );
    let output = run_basic_repl(input.as_bytes());
    assert!(
        output.status.success(),
        "basic repl should exit successfully; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("default> "));
    assert!(stdout.contains("Commands"));
    assert!(stdout.contains("help"));
    assert!(stdout.contains("exit"));
    assert!(
        stdout.matches("rose-pine-moon").count() == 2,
        "source and recall should each execute the resolved config command; stdout:\n{stdout}"
    );
    assert!(stderr.contains("Warning: input is not a terminal"));

    let mut documents = Vec::new();
    let mut remaining = stdout.as_ref();
    while let Some(start) = remaining.find("[\n") {
        let mut document = serde_json::Deserializer::from_str(&remaining[start..])
            .into_iter::<serde_json::Value>();
        documents.push(
            document
                .next()
                .unwrap()
                .expect("each sourced or replayed result should be JSON"),
        );
        remaining = &remaining[start + document.byte_offset()..];
    }
    assert_eq!(
        documents[0],
        serde_json::json!([{"id": "dracula", "name": "Dracula"}])
    );
    let selected = serde_json::json!([{"id": "nord", "name": "Nord"}]);
    assert_eq!(documents[1], selected);
    assert_eq!(
        documents[2], selected,
        "last should replay the saved pipeline"
    );
    assert_eq!(documents[3][0]["id"], "nord");
    assert_eq!(documents[3][0]["name"], "Nord");
    assert_eq!(
        documents[3][0]["accent"], "#88c0d0",
        "raw replay should retain the theme palette"
    );
    let before_failure = serde_json::json!([{"id": "dracula", "name": "Dracula"}]);
    assert_eq!(documents[4], before_failure);
    assert_eq!(
        documents[5], before_failure,
        "a stopped batch should retain its last completed result for replay"
    );
    assert_eq!(documents[6], before_failure);
    let recovered = serde_json::json!([{"id": "nord", "name": "Nord"}]);
    assert_eq!(documents[7], recovered);
    assert_eq!(
        documents[8], recovered,
        "continuing after a failed batch operation should commit the final result for replay"
    );
    assert_eq!(documents.len(), 9);
    assert!(
        stderr.contains(&format!("{}:2", recovery.display())),
        "{stderr}"
    );
}

#[test]
fn repl_sources_config_audits_and_replays_text_and_json_exports() {
    let files = make_temp_dir("osp-cli-repl-config-audit");
    for format in ["value", "json"] {
        let audit = files.path().join(format!("{format} audit.osp"));
        std::fs::write(&audit, "config explain ui.width\n").unwrap();
        let mut input = format!(
            "config set --session repl.simple_prompt true\nconfig set --session ui.width 96\nconfig set --session ui.format {format}\nsource '{}'\nlast\nlast --raw\n",
            audit.display(),
        );
        if format == "json" {
            input.push_str("config show --sources | F key=ui.width | P key,value,source,scope_profile,scope_terminal\nlast\nlast --raw\n");
        }
        input.push_str("exit\n");
        let output = run_basic_repl(input.as_bytes());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{format}: {stderr}\n{stdout}");
        let frames = stdout.split("default> ").skip(1).collect::<Vec<_>>();
        let frame = |index: usize| {
            frames
                .get(index)
                .unwrap_or_else(|| {
                    panic!("{format}: missing command result {index}:\n{stdout}\n{stderr}")
                })
                .trim()
        };
        if format == "json" {
            let document = |index| {
                serde_json::from_str::<serde_json::Value>(frame(index)).unwrap_or_else(|error| {
                    panic!("{format}: result {index}: {error}\n{stdout}\n{stderr}")
                })
            };
            let raw = document(3);
            assert_eq!(raw["key"], "ui.width");
            assert_eq!(raw["value"], 96);
            assert_eq!(raw["value_type"], "integer");
            assert_eq!(raw["source"], "session");
            assert_eq!(raw["scope"], "profile:default");
            assert_eq!(raw["active_profile"], "default");
            assert_eq!(raw["terminal"], "repl");
            let winner = raw["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| candidate["winner"] == true)
                .expect("raw audit should retain the winning candidate");
            assert_eq!(winner["value"], 96);
            assert_eq!(winner["source"], "session");
            assert_eq!(winner["scope"], "profile:default");
            assert_eq!(
                document(4),
                raw,
                "last should retain the complete JSON audit"
            );
            assert_eq!(document(5), raw, "raw JSON audit replay should agree");
            let projection = serde_json::json!([{
                "key": "ui.width", "value": 96, "source": "session",
                "scope_profile": "default", "scope_terminal": null
            }]);
            assert_eq!(
                document(6),
                projection,
                "config export should retain typed source metadata"
            );
            assert_eq!(
                document(7),
                projection,
                "last should repeat the config export pipeline"
            );
            let raw_export = document(8);
            let exported_width = raw_export
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["key"] == "ui.width")
                .expect("raw export should retain the selected config value");
            assert_eq!(exported_width["value"], 96);
            assert_eq!(exported_width["source"], "session");
            assert_eq!(exported_width["scope_profile"], "default");
            assert_eq!(exported_width["scope_terminal"], serde_json::Value::Null);
        } else {
            let raw = frame(3);
            assert!(
                raw.lines().any(|line| line.trim() == "key: ui.width"),
                "{raw}"
            );
            assert!(
                raw.lines().any(|line| line.trim() == "value: 96 (integer)"),
                "{raw}"
            );
            assert!(
                raw.lines().any(|line| line.trim() == "source: session"),
                "{raw}"
            );
            assert!(
                raw.lines().any(|line| line.trim() == "terminal: repl"),
                "{raw}"
            );
            assert_eq!(frame(4), raw, "last should retain the complete text audit");
            assert_eq!(frame(5), raw, "raw text audit replay should agree");
        }
    }
}

#[test]
fn repl_basic_mode_exits_cleanly_on_immediate_eof() {
    let output = run_basic_repl(b"");
    assert!(
        output.status.success(),
        "basic repl should exit cleanly on EOF; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("default> "));
    assert!(stderr.contains("Warning: input is not a terminal"));
}

#[test]
fn repl_basic_mode_restarts_after_refresh_without_tty() {
    let files = make_temp_dir("osp-cli-repl-source-refresh");
    let commands = files.path().join("refresh.osp");
    std::fs::write(&commands, "plugins refresh\n").unwrap();
    let input = format!(
        "source '{}'\ntheme show dracula --json\nexit\n",
        commands.display()
    );
    let output = run_basic_repl(input.as_bytes());
    assert!(
        output.status.success(),
        "basic repl restart path should succeed; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let prompt_count = stdout.matches("default> ").count();
    let warning_count = stderr.matches("Warning: input is not a terminal").count();

    assert!(
        prompt_count >= 2,
        "expected refresh to restart and render the prompt again; stdout:\n{stdout}"
    );
    assert!(
        warning_count >= 1,
        "expected basic-mode fallback warning; stderr:\n{stderr}"
    );
    assert!(stderr.contains(&format!(
        "{}:1: command requires a REPL restart",
        commands.display()
    )));
    let start = stdout
        .find("[\n")
        .expect("command after source reload should return JSON");
    let theme = serde_json::Deserializer::from_str(&stdout[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(theme[0]["id"], "dracula");
    assert_eq!(theme[0]["name"], "Dracula");
}
